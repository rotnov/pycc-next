//! Type-parameter and protocol substitution through types, statements and bodies.

use super::*;

/// D-133: a `Ty`-tree rewrite substituting a top-level `Ty::Param(param_name)`
/// occurrence with `concrete`, leaving every other `Ty` shape untouched. Not
/// a *recursive* container walk: `generic_type_param_name` (called by every
/// public entry point before this function ever runs, both here and in
/// `check_generic_function`) already rejects any container-position
/// `Ty::Param` with `T0042`, and since D-228 (issue #918) container
/// *annotation* lowering rejects one earlier still, in `pycc_hir`, with the
/// same code -- deliberately, precisely because this function is not
/// recursive. So by the time `substitute_ty` runs, `ty` is
/// provably either a bare `Ty::Param(param_name)` or a shape that can never
/// contain one nested inside `List`/`Set`/`Dict`/`Tuple` -- recursing into
/// those container variants here would be untestable dead code (D-014's
/// hard coverage gate has no exemption for it), not genuine generality.
pub(crate) fn substitute_ty(ty: &Ty, param_name: &str, concrete: &Ty) -> Ty {
    match ty {
        Ty::Param(name) if name.as_ref() == param_name => concrete.clone(),
        other => other.clone(),
    }
}

/// #380 (PR-20): Like `substitute_ty` but substitutes
/// `Ty::Protocol(protocol_name)` with `concrete` (a `Ty::Instance`).
/// Used to monomorphize functions with protocol-typed parameters — at
/// each call site, the protocol type is replaced with the concrete
/// argument's class type, so MIR/codegen can resolve method calls and
/// attribute access against the concrete class.
pub(crate) fn substitute_ty_protocol(ty: &Ty, protocol_name: &str, concrete: &Ty) -> Ty {
    match ty {
        Ty::Protocol(name) if name.as_ref() == protocol_name => concrete.clone(),
        other => other.clone(),
    }
}

/// #380 (PR-20): Mangles a function name for protocol monomorphization
/// with one or more protocol→concrete substitutions, following the
/// existing `0gen_` convention. Each substitution appends
/// `__{protocol_name}_{concrete_name}` to the mangled name.
pub(crate) fn mangle_protocol_instantiation(
    func_name: &str,
    substitutions: &[(String, Ty)],
) -> String {
    let mut name = format!("0gen_{func_name}");
    for (proto_name, concrete) in substitutions {
        if let Ty::Instance(concrete_name) = concrete {
            name.push_str(&format!("__{proto_name}_{concrete_name}"));
        }
    }
    name
}

pub(crate) fn substitute_stmt_protocols(stmt: &HirStmt, substitutions: &[(String, Ty)]) -> HirStmt {
    match stmt {
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final,
        } => HirStmt::AnnAssign {
            target: target.clone(),
            annotation: substitute_ty_protocols(annotation, substitutions),
            value: value.clone(),
            is_final: *is_final,
        },
        HirStmt::If { test, body, orelse } => HirStmt::If {
            test: test.clone(),
            body: substitute_body_protocols(body, substitutions),
            orelse: substitute_body_protocols(orelse, substitutions),
        },
        HirStmt::While { test, body } => HirStmt::While {
            test: test.clone(),
            body: substitute_body_protocols(body, substitutions),
        },
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => HirStmt::ForRange {
            var: var.clone(),
            start: start.clone(),
            stop: stop.clone(),
            step: step.clone(),
            body: substitute_body_protocols(body, substitutions),
        },
        HirStmt::ForList { var, list, body } => HirStmt::ForList {
            var: var.clone(),
            list: list.clone(),
            body: substitute_body_protocols(body, substitutions),
        },
        HirStmt::LoopElse { loop_stmt, orelse } => HirStmt::LoopElse {
            loop_stmt: Box::new(substitute_stmt_protocols(loop_stmt, substitutions)),
            orelse: substitute_body_protocols(orelse, substitutions),
        },
        other => other.clone(),
    }
}

/// PEP 695 (#387): Like `substitute_ty` but also substitutes
/// `Ty::Instance(class_name)` with `Ty::Instance(mangled_class)` — needed
/// when monomorphizing a generic class's methods, where `self`'s own
/// parameter type (`Ty::Instance("Container")`) must become
/// `Ty::Instance("0gen_Container__T_int")` so MIR/codegen resolve attribute
/// slots against the monomorphized class's substituted attribute types, not
/// the original generic class's `Ty::Param`-typed slots.
pub(crate) fn substitute_ty_with_class(
    ty: &Ty,
    param_name: &str,
    concrete: &Ty,
    class_name: &str,
    mangled_class: &str,
) -> Ty {
    match ty {
        Ty::Param(name) if name.as_ref() == param_name => concrete.clone(),
        Ty::Instance(name) if name.as_ref() == class_name => {
            Ty::Instance(Box::new(mangled_class.to_string()))
        }
        other => other.clone(),
    }
}

/// D-133: clones a generic function's body, substituting only the `Ty`
/// annotations `substitute_ty` above would rewrite -- `HirStmt::AnnAssign`'s
/// `annotation` field is the only place an embedded `Ty` appears inside a
/// `HirStmt`/`HirExpr` tree (every `HirExpr` variant is untyped; type is
/// always inferred contextually), so every other statement shape is cloned
/// unchanged, recursing only into nested bodies (`If`/`While`/`ForRange`/
/// `ForList`) to reach a nested `AnnAssign`.
pub(crate) fn substitute_body(body: &[HirStmt], param_name: &str, concrete: &Ty) -> Vec<HirStmt> {
    body.iter()
        .map(|stmt| substitute_stmt(stmt, param_name, concrete))
        .collect()
}

pub(crate) fn substitute_stmt(stmt: &HirStmt, param_name: &str, concrete: &Ty) -> HirStmt {
    match stmt {
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final,
        } => HirStmt::AnnAssign {
            target: target.clone(),
            annotation: substitute_ty(annotation, param_name, concrete),
            value: value.clone(),
            is_final: *is_final,
        },
        HirStmt::If { test, body, orelse } => HirStmt::If {
            test: test.clone(),
            body: substitute_body(body, param_name, concrete),
            orelse: substitute_body(orelse, param_name, concrete),
        },
        HirStmt::While { test, body } => HirStmt::While {
            test: test.clone(),
            body: substitute_body(body, param_name, concrete),
        },
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => HirStmt::ForRange {
            var: var.clone(),
            start: start.clone(),
            stop: stop.clone(),
            step: step.clone(),
            body: substitute_body(body, param_name, concrete),
        },
        HirStmt::ForList { var, list, body } => HirStmt::ForList {
            var: var.clone(),
            list: list.clone(),
            body: substitute_body(body, param_name, concrete),
        },
        HirStmt::LoopElse { loop_stmt, orelse } => HirStmt::LoopElse {
            loop_stmt: Box::new(substitute_stmt(loop_stmt, param_name, concrete)),
            orelse: substitute_body(orelse, param_name, concrete),
        },
        other => other.clone(),
    }
}

/// PEP 695 (#387): Like `substitute_body` but uses `substitute_ty_with_class`
/// to also rewrite `Ty::Instance(class_name)` to `Ty::Instance(mangled_class)`.
pub(crate) fn substitute_body_with_class(
    body: &[HirStmt],
    param_name: &str,
    concrete: &Ty,
    class_name: &str,
    mangled_class: &str,
) -> Vec<HirStmt> {
    body.iter()
        .map(|stmt| {
            substitute_stmt_with_class(stmt, param_name, concrete, class_name, mangled_class)
        })
        .collect()
}

pub(crate) fn substitute_stmt_with_class(
    stmt: &HirStmt,
    param_name: &str,
    concrete: &Ty,
    class_name: &str,
    mangled_class: &str,
) -> HirStmt {
    match stmt {
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final,
        } => HirStmt::AnnAssign {
            target: target.clone(),
            annotation: substitute_ty_with_class(
                annotation,
                param_name,
                concrete,
                class_name,
                mangled_class,
            ),
            value: value.clone(),
            is_final: *is_final,
        },
        HirStmt::If { test, body, orelse } => HirStmt::If {
            test: test.clone(),
            body: substitute_body_with_class(body, param_name, concrete, class_name, mangled_class),
            orelse: substitute_body_with_class(
                orelse,
                param_name,
                concrete,
                class_name,
                mangled_class,
            ),
        },
        HirStmt::While { test, body } => HirStmt::While {
            test: test.clone(),
            body: substitute_body_with_class(body, param_name, concrete, class_name, mangled_class),
        },
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => HirStmt::ForRange {
            var: var.clone(),
            start: start.clone(),
            stop: stop.clone(),
            step: step.clone(),
            body: substitute_body_with_class(body, param_name, concrete, class_name, mangled_class),
        },
        HirStmt::ForList { var, list, body } => HirStmt::ForList {
            var: var.clone(),
            list: list.clone(),
            body: substitute_body_with_class(body, param_name, concrete, class_name, mangled_class),
        },
        HirStmt::LoopElse { loop_stmt, orelse } => HirStmt::LoopElse {
            loop_stmt: Box::new(substitute_stmt_with_class(
                loop_stmt,
                param_name,
                concrete,
                class_name,
                mangled_class,
            )),
            orelse: substitute_body_with_class(
                orelse,
                param_name,
                concrete,
                class_name,
                mangled_class,
            ),
        },
        other => other.clone(),
    }
}
