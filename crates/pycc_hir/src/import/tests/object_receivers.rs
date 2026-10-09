//! Issue #1095: in a module that can hold a CPython object -- an `ext`
//! module (D-258), one whose body binds a foreign import anywhere (D-244
//! rule 3; for the whole module since #1482), or (#1425) one with a direct
//! project dependency that can hold one -- a call
//! to `append`, `pop`, `get` or `add` keeps both readings in a
//! `HirExpr::ReceiverDispatchedCall`, so an object receiver can take the
//! foreign method-call reading. Every other module lowers these calls
//! exactly as before. Issue #1425 also publishes the module's final state
//! as `LoweredModule::object_receivers`, which importers inherit.

use super::*;
use crate::{ContainerFallback, HirExpr, HirItem, HirStmt, Ty, receiver_takes_method_path};

/// Lowers `source` as an `ext` module (`ext == true`) or a `native` one,
/// with every import request answered `Foreign`.
fn lower(source: &str, ext: bool) -> LoweredModule {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    resolved.set_ext_module(ext);
    for request in project_import_requests(&parsed) {
        resolved.insert(request.span, ResolvedImport::Foreign);
    }
    lower_module(&parsed, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
}

/// Lowers `source` as a `native` module whose dependencies can
/// (`inherited == true`) or cannot hold a CPython object (#1425). No import
/// request is answered, so nothing in `source` admits on its own.
fn lower_inheriting(source: &str, inherited: bool) -> LoweredModule {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    resolved.inherit_object_receivers(inherited);
    lower_module(&parsed, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
}

/// Every top-level expression statement of `module`, in source order.
fn top_level_exprs(module: &LoweredModule) -> Vec<&HirExpr> {
    module
        .hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::TopLevelStmt(HirStmt::ExprStmt(expr)) => Some(expr),
            _ => None,
        })
        .collect()
}

/// The expression statements of the module-level function `name`'s body.
fn function_exprs<'m>(module: &'m LoweredModule, name: &str) -> Vec<&'m HirExpr> {
    let body = module
        .hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name: n, body, .. } if n == name => Some(body),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no function `{name}`"));
    body.iter()
        .filter_map(|stmt| match stmt {
            HirStmt::ExprStmt(expr) => Some(expr),
            _ => None,
        })
        .collect()
}

/// The container fallback of a receiver-dispatched call, after checking its
/// method reading names `method`.
fn dispatched(expr: &HirExpr, method: &str) -> ContainerFallback {
    let HirExpr::ReceiverDispatchedCall { call, container } = expr else {
        panic!("expected a receiver-dispatched call, got {expr:?}");
    };
    assert_eq!(call.method_receiver().map(|(_, m)| m), Some(method));
    container.clone()
}

#[test]
fn an_object_receiver_takes_the_method_reading() {
    assert!(receiver_takes_method_path(&Ty::Object));
    for native in [
        Ty::List(Box::new(Ty::Int)),
        Ty::Dict(Box::new((Ty::Str, Ty::Int))),
        Ty::Set(Box::new(Ty::Int)),
        Ty::Int,
    ] {
        assert!(!receiver_takes_method_path(&native), "{native:?}");
    }
}

#[test]
fn an_ext_module_dispatches_all_four_names_on_their_receiver() {
    let module = lower(
        "from typing import Any\n\n\ndef f(o: Any) -> None:\n    o.append(1)\n    o.pop()\n    \
         o.get(1, 2)\n    o.add(1)\n    o.get(1)\n    o.pop(0)\n    o.extend(o)\n",
        true,
    );
    let exprs = function_exprs(&module, "f");
    for (expr, method) in exprs.iter().zip(["append", "pop", "get", "add"]) {
        assert_eq!(dispatched(expr, method), ContainerFallback::Admitted);
    }
    // The arities the container fast path refuses keep that refusal beside
    // the method reading, which an object receiver takes instead.
    for (expr, method) in exprs[4..6].iter().zip(["get", "pop"]) {
        assert!(matches!(
            dispatched(expr, method),
            ContainerFallback::Refused(_)
        ));
    }
    // A name the container fast paths never claim is an ordinary call.
    assert!(matches!(exprs[6], HirExpr::MethodCall { method, .. } if method == "extend"));
}

#[test]
fn a_foreign_import_turns_the_gate_on_for_the_whole_module() {
    let module = lower(
        "xs = [1]\nxs.append(2)\nimport gc\ngc.get(1)\nxs.append(3)\ngc.pop()\n",
        false,
    );
    let exprs = top_level_exprs(&module);
    // Issue #1482: a call above the import keeps both readings too.
    assert_eq!(dispatched(exprs[0], "append"), ContainerFallback::Admitted);
    assert!(module.object_receivers);
    assert!(matches!(
        dispatched(exprs[1], "get"),
        ContainerFallback::Refused(_)
    ));
    assert_eq!(dispatched(exprs[2], "append"), ContainerFallback::Admitted);
    assert_eq!(dispatched(exprs[3], "pop"), ContainerFallback::Admitted);
}

#[test]
fn a_native_module_without_a_foreign_import_is_unchanged() {
    let module = lower(
        "xs = [1]\nxs.append(2)\nxs.pop()\nd = {'a': 1}\nd.get('a', 2)\ns = {1}\ns.add(2)\n",
        false,
    );
    let exprs = top_level_exprs(&module);
    assert!(matches!(exprs[0], HirExpr::ListAppend { .. }), "{exprs:?}");
    assert!(matches!(exprs[1], HirExpr::ListPop { .. }), "{exprs:?}");
    assert!(
        matches!(exprs[2], HirExpr::DictGetOrDefault { .. }),
        "{exprs:?}"
    );
    assert!(matches!(exprs[3], HirExpr::SetAdd { .. }), "{exprs:?}");
}

#[test]
fn a_foreign_import_inside_a_module_level_block_turns_the_gate_on() {
    let module = lower(
        "try:\n    import gc\nexcept ImportError:\n    pass\ngc.get(1)\n",
        false,
    );
    let exprs = top_level_exprs(&module);
    assert!(matches!(
        dispatched(exprs[0], "get"),
        ContainerFallback::Refused(_)
    ));
}

#[test]
fn an_inherited_admission_dispatches_from_the_first_statement() {
    let module = lower_inheriting("xs = [1]\nxs.append(2)\n", true);
    let exprs = top_level_exprs(&module);
    assert_eq!(dispatched(exprs[0], "append"), ContainerFallback::Admitted);
    // The module publishes what it inherited, so the bit is transitive.
    assert!(module.object_receivers);
}

#[test]
fn an_inherited_false_changes_nothing() {
    let mut resolved = ResolvedImports::default();
    resolved.inherit_object_receivers(false);
    assert!(!resolved.object_receivers());
    let module = lower_inheriting("xs = [1]\nxs.append(2)\n", false);
    let exprs = top_level_exprs(&module);
    assert!(matches!(exprs[0], HirExpr::ListAppend { .. }), "{exprs:?}");
    assert!(!module.object_receivers);
}

#[test]
fn an_inherited_true_is_not_cleared_by_a_later_false() {
    let mut resolved = ResolvedImports::default();
    resolved.inherit_object_receivers(true);
    resolved.inherit_object_receivers(false);
    assert!(resolved.object_receivers());
}

#[test]
fn an_own_foreign_import_publishes_the_final_state() {
    // The import comes last; the admission is decided before the first
    // statement is lowered (#1482), so the published bit is set as well.
    let module = lower("xs = [1]\nimport gc\n", false);
    assert!(module.object_receivers);
}

#[test]
fn a_block_foreign_import_publishes_the_final_state() {
    let module = lower(
        "try:\n    import gc\nexcept ImportError:\n    pass\n",
        false,
    );
    assert!(module.object_receivers);
}

#[test]
fn a_native_module_without_a_foreign_import_publishes_false() {
    let module = lower("xs = [1]\nxs.append(2)\n", false);
    assert!(!module.object_receivers);
}

/// Issue #1482: a `def` written above the module's own foreign import
/// dispatches, since CPython reads `gc` only when the function runs.
#[test]
fn a_def_above_the_import_dispatches() {
    let module = lower(
        "def f() -> None:\n    gc.garbage.append(1)\n\n\nimport gc\n",
        false,
    );
    let exprs = function_exprs(&module, "f");
    assert_eq!(dispatched(exprs[0], "append"), ContainerFallback::Admitted);
}

#[test]
fn a_block_import_below_admits_code_above_it() {
    let module = lower(
        "xs = [1]\nxs.append(2)\ntry:\n    import gc\nexcept ImportError:\n    pass\n",
        false,
    );
    let exprs = top_level_exprs(&module);
    assert_eq!(dispatched(exprs[0], "append"), ContainerFallback::Admitted);
}

/// A foreign `from ... import` nested in a module-level `try` (#1383)
/// admits a `def` written above it as well.
#[test]
fn a_block_from_import_below_admits_a_def_above_it() {
    let module = lower(
        "def f() -> None:\n    garbage.append(1)\n\n\ntry:\n    from gc import garbage\n\
         except ImportError:\n    raise\n",
        false,
    );
    let exprs = function_exprs(&module, "f");
    assert_eq!(dispatched(exprs[0], "append"), ContainerFallback::Admitted);
    assert!(module.object_receivers);
}

/// An import in an `if TYPE_CHECKING:` body never runs, so it never admits,
/// although `lower` answers its request `Foreign`.
#[test]
fn a_type_checking_import_does_not_admit() {
    let module = lower(
        "from typing import TYPE_CHECKING\nxs = [1]\nxs.append(2)\nif TYPE_CHECKING:\n    \
         import gc\n",
        false,
    );
    let exprs = top_level_exprs(&module);
    assert!(matches!(exprs[0], HirExpr::ListAppend { .. }), "{exprs:?}");
    assert!(!module.object_receivers);
}

/// An aliased guard is recognized only through the import table built so
/// far, so the pre-scan must replay that table.
#[test]
fn an_aliased_type_checking_import_does_not_admit() {
    let module = lower(
        "import typing as t\nxs = [1]\nxs.append(2)\nif t.TYPE_CHECKING:\n    import gc\n",
        false,
    );
    let exprs = top_level_exprs(&module);
    assert!(matches!(exprs[0], HirExpr::ListAppend { .. }), "{exprs:?}");
    assert!(!module.object_receivers);
}

/// `t` is `typing` at the guard and `math` only afterwards, so the guard
/// folds exactly as the item loop folds it: a whole-module import table
/// would read `t` as `math`, walk the body and admit.
#[test]
fn a_rebound_typing_alias_is_read_positionally() {
    let module = lower(
        "import typing as t\nxs = [1]\nxs.append(2)\nif t.TYPE_CHECKING:\n    import gc\n\
         import math as t\nprint(t.sqrt(4.0))\n",
        false,
    );
    let exprs = top_level_exprs(&module);
    assert!(matches!(exprs[0], HirExpr::ListAppend { .. }), "{exprs:?}");
    assert!(!module.object_receivers);
}

/// An import statement the item loop refuses is skipped by the pre-scan,
/// which goes on to a foreign import below it, and the module still
/// reports the refusal.
#[test]
fn a_refused_import_is_skipped_by_the_pre_scan() {
    let parsed = parse("xs = [1]\nfrom __future__ import annotations\nimport gc\n");
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        resolved.insert(request.span, ResolvedImport::Foreign);
    }
    assert!(crate::module::own_foreign::binds_foreign_import(
        &parsed,
        &resolved,
        &[]
    ));
    let parsed = parse("xs = [1]\nfrom __future__ import annotations\n");
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        resolved.insert(request.span, ResolvedImport::Foreign);
    }
    let diagnostics = lower_module(&parsed, &resolved, None)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "L0001", "{diagnostics:#?}");
}
