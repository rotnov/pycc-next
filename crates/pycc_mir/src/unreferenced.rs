//! Compiler-generated functions nothing calls are not emitted.
//!
//! Codegen gives every `MirItem::Function` a `fnptr_` slot that the module
//! body stores its address into, so a function no code calls still reaches
//! the object. Most functions are bound to a Python name and may be called
//! from anywhere, but a few are compiler spellings only compiled code calls
//! by name ([`INTERNAL`]): a native multi-value twin (`crate::multi_return`)
//! and a region's native copies and helpers
//! (`pycc_ast::specialize_regions`). One of those that no kept function or
//! module statement calls is dropped ([`drop_unreferenced`]): a copy whose
//! every caller unpacks its result calls its twin, and the copy itself is
//! then dead.
//!
//! A reference is a call (`MirExpr::Call`), a function value
//! (`MirExpr::MakeClosure`) or a name (`MirExpr::Name`), and an unpacking
//! call's twin and its fallback (`MirStmt::AssignMulti`). Codegen calls a
//! native twin whenever the module has it, so the call of the original in
//! its fallback names no reference then, though that call's arguments do;
//! an original that is dropped is not looked up (`pycc_codegen::multi_out`).
//! An object twin's fallback is always emitted and counts whole.

use crate::{AssignMulti, MirExpr, MirItem, MirModule, MirStmt};
use std::collections::BTreeSet;

/// The prefixes of the functions only compiled code calls, by name: a
/// native multi-value twin (`crate::multi_return::twin_name`; an object
/// twin, `0gen_multi2_...`, is reached by a name codegen derives and
/// stays), a region's native copy (`__pycc_rnative_`), its generic arms'
/// conversions (`__pycc_rconv_`) and calls (`__pycc_rslow`).
const INTERNAL: [&str; 4] = [
    "0gen_multi_",
    pycc_hir::REGION_COPY_PREFIX,
    "__pycc_rconv_",
    pycc_hir::REGION_SLOW_PREFIX,
];

/// Whether `name` is a function only compiled code calls ([`INTERNAL`]).
#[must_use]
fn is_internal(name: &str) -> bool {
    INTERNAL.iter().any(|prefix| name.starts_with(prefix))
}

/// Drops the [`INTERNAL`] functions of `module` that nothing kept refers to.
pub(crate) fn drop_unreferenced(module: &mut MirModule) {
    let functions: BTreeSet<&str> = module
        .items
        .iter()
        .filter_map(|item| match item {
            MirItem::Function { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    if !functions.iter().any(|name| is_internal(name)) {
        return;
    }
    let mut kept: BTreeSet<String> = BTreeSet::new();
    let mut pending: Vec<&[MirStmt]> = Vec::new();
    for item in &module.items {
        match item {
            MirItem::Function { name, body, .. } if !is_internal(name) => pending.push(body),
            MirItem::TopLevelStmt(stmt) => pending.push(std::slice::from_ref(stmt)),
            _ => {}
        }
    }
    let bodies = |name: &str| {
        module.items.iter().find_map(|item| match item {
            MirItem::Function { name: n, body, .. } if n == name => Some(body.as_slice()),
            _ => None,
        })
    };
    while let Some(body) = pending.pop() {
        let mut found = Vec::new();
        names_in(body, &functions, &mut found);
        for name in found {
            if is_internal(&name) && kept.insert(name.clone()) {
                pending.extend(bodies(&name));
            }
        }
    }
    module.items.retain(|item| match item {
        MirItem::Function { name, .. } => !is_internal(name) || kept.contains(name),
        _ => true,
    });
}

/// The functions `body` refers to, into `out`.
fn names_in(body: &[MirStmt], functions: &BTreeSet<&str>, out: &mut Vec<String>) {
    for stmt in body {
        if let MirStmt::AssignMulti(multi) = stmt {
            out.push(multi.twin.clone());
            for expr in &multi.args {
                expr_names(expr, out);
            }
            fallback_names(multi, functions, out);
            continue;
        }
        for expr in stmt.exprs() {
            expr_names(expr, out);
        }
        for inner in stmt.bodies() {
            names_in(inner, functions, out);
        }
    }
}

/// The functions an `AssignMulti`'s fallback refers to, into `out`.
///
/// An object twin's (`crate::multi_return_object`) has no `args`: its
/// fallback holds the call with its arguments and codegen always emits it
/// (`pycc_codegen::multi_out`), so all of it counts. A native twin's
/// fallback repeats `args` in a call of the original, which codegen makes
/// only when the original is in the module; while the twin is, that call's
/// callee is not a reference, but its arguments and the rest of the
/// fallback still are.
fn fallback_names(multi: &AssignMulti, functions: &BTreeSet<&str>, out: &mut Vec<String>) {
    let native_twin = crate::multi_return_object::twin_arity(&multi.twin).is_none()
        && functions.contains(multi.twin.as_str());
    let Some((first, rest)) = multi.fallback.split_first() else {
        return;
    };
    match first {
        MirStmt::Assign { value, .. } if native_twin => {
            let call = match value {
                MirExpr::ObjUnpack { value, .. } => value,
                value => value,
            };
            match call {
                MirExpr::Call { callee, args, .. } if *callee == multi.original => {
                    for arg in args {
                        expr_names(arg, out);
                    }
                }
                call => expr_names(call, out),
            }
        }
        first => names_in(std::slice::from_ref(first), functions, out),
    }
    names_in(rest, functions, out);
}

fn expr_names(expr: &MirExpr, out: &mut Vec<String>) {
    match expr {
        MirExpr::Call { callee, .. } => out.push(callee.clone()),
        MirExpr::MakeClosure { function, .. } => out.push(function.clone()),
        MirExpr::Name { name, .. } => out.push(name.clone()),
        _ => {}
    }
    for child in expr.children() {
        expr_names(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ty;

    fn function(name: &str, body: Vec<MirStmt>) -> MirItem {
        MirItem::Function {
            name: name.to_string(),
            params: Vec::new(),
            return_ty: Ty::Int,
            body,
        }
    }

    fn call(callee: &str) -> MirExpr {
        MirExpr::Call {
            callee: callee.to_string(),
            args: Vec::new(),
            ty: Ty::Int,
        }
    }

    fn names(module: &MirModule) -> Vec<&str> {
        module
            .items
            .iter()
            .filter_map(|item| match item {
                MirItem::Function { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    fn unpack(original: &str, twin: &str) -> MirStmt {
        MirStmt::AssignMulti(Box::new(AssignMulti {
            original: original.to_string(),
            twin: twin.to_string(),
            args: Vec::new(),
            temps: Vec::new(),
            fallback: vec![MirStmt::Assign {
                target: "t".to_string(),
                value: call(original),
            }],
        }))
    }

    #[test]
    fn only_internal_functions_nothing_reaches_are_dropped() {
        let mut module = MirModule {
            items: vec![
                // A user function nothing calls stays: Python may call it.
                function("unused", Vec::new()),
                function("f", vec![MirStmt::Return(Some(call("__pycc_rnative_a")))]),
                // Called from a kept copy, through a chain.
                function("__pycc_rnative_a", vec![MirStmt::ExprStmt(call("__pycc_rconv_i"))]),
                function("__pycc_rconv_i", Vec::new()),
                // Called only by a dead copy.
                function("__pycc_rnative_dead", vec![MirStmt::ExprStmt(call("__pycc_rslow0"))]),
                function("__pycc_rslow0", Vec::new()),
                // An object twin stays: codegen derives its name.
                function("0gen_multi2___pycc_lift1_g__", Vec::new()),
            ],
            class_defs: Default::default(),
        };
        drop_unreferenced(&mut module);
        assert_eq!(
            names(&module),
            ["unused", "f", "__pycc_rnative_a", "__pycc_rconv_i", "0gen_multi2___pycc_lift1_g__"]
        );
    }

    #[test]
    fn an_unpacked_copy_keeps_its_twin_and_not_itself() {
        let mut module = MirModule {
            items: vec![
                function("f", vec![unpack("__pycc_rnative_a", "0gen_multi___pycc_rnative_a")]),
                function("__pycc_rnative_a", Vec::new()),
                function("0gen_multi___pycc_rnative_a", Vec::new()),
                // Without its twin, the fallback calls the original.
                function("g", vec![unpack("__pycc_rnative_b", "0gen_multi___pycc_rnative_b")]),
                function("__pycc_rnative_b", Vec::new()),
            ],
            class_defs: Default::default(),
        };
        drop_unreferenced(&mut module);
        assert_eq!(
            names(&module),
            ["f", "0gen_multi___pycc_rnative_a", "g", "__pycc_rnative_b"]
        );
    }

    /// An unpacking whose call's arguments are only in its fallback.
    fn unpack_with(original: &str, twin: &str, arg: &str, object: bool) -> MirStmt {
        let made = MirExpr::Call {
            callee: original.to_string(),
            args: vec![call(arg)],
            ty: Ty::Object,
        };
        let value = if object {
            MirExpr::ObjUnpack {
                value: Box::new(made),
                arity: 2,
            }
        } else {
            made
        };
        MirStmt::AssignMulti(Box::new(AssignMulti {
            original: original.to_string(),
            twin: twin.to_string(),
            args: Vec::new(),
            temps: Vec::new(),
            fallback: vec![
                MirStmt::Assign {
                    target: "t".to_string(),
                    value,
                },
                MirStmt::ExprStmt(call("__pycc_rconv_i")),
            ],
        }))
    }

    #[test]
    fn a_fallbacks_arguments_and_rest_are_references() {
        let mut module = MirModule {
            items: vec![
                // An object twin's fallback is always emitted: its call's
                // arguments keep the slow call and the copy they name.
                function(
                    "f",
                    vec![unpack_with("__pycc_lift1_g__", "0gen_multi2___pycc_lift1_g__", "__pycc_rslow1", true)],
                ),
                function("__pycc_lift1_g__", Vec::new()),
                function("0gen_multi2___pycc_lift1_g__", Vec::new()),
                function("__pycc_rslow1", Vec::new()),
                // A native twin's fallback: the original is not called
                // while the twin is there, but the arguments still count.
                function(
                    "h",
                    vec![unpack_with("__pycc_rnative_c", "0gen_multi___pycc_rnative_c", "__pycc_rnative_d", false)],
                ),
                function("__pycc_rnative_c", Vec::new()),
                function("0gen_multi___pycc_rnative_c", Vec::new()),
                function("__pycc_rnative_d", Vec::new()),
                function("__pycc_rconv_i", Vec::new()),
            ],
            class_defs: Default::default(),
        };
        drop_unreferenced(&mut module);
        assert_eq!(
            names(&module),
            [
                "f",
                "__pycc_lift1_g__",
                "0gen_multi2___pycc_lift1_g__",
                "__pycc_rslow1",
                "h",
                "0gen_multi___pycc_rnative_c",
                "__pycc_rnative_d",
                "__pycc_rconv_i",
            ]
        );
    }
}
