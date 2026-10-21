//! Nested functions used as values: which may be, and the expression that
//! makes the value.
//!
//! A nested function read as a value (`return add`, `apply(add, 3)`)
//! becomes `__pycc_closure__(lifted, captures...)`: the lifted function
//! with its captured enclosing variables bound when the value is made
//! (`pycc_hir::CLOSURE_MARKER`). CPython's closure reads each variable when
//! it *runs*, so binding it early is the same only when the variable cannot
//! change after the value is made and is already bound when it is made: a
//! parameter the enclosing function never rebinds, or a variable one
//! top-level statement before the `def` assigns and nothing else binds (by
//! any binding form, see `Bindings`), with every value use of the function
//! after its `def`. A variable a nested function writes (`nonlocal`) is
//! captured as its cell, which both sides share. A function capturing
//! anything else, with parameter defaults, or with an unannotated
//! parameter (a function value's type is its annotations) is not used as
//! a value: it stays nested, and the compiler reports it as before.

use ruff_python_ast::{Expr, Stmt, StmtFunctionDef};
use std::collections::{HashMap, HashSet};

/// Whether the nested function `def` may be made a value: its own shape.
pub(super) fn value_shape(def: &StmtFunctionDef) -> bool {
    // Non-strict mode: an unannotated parameter is a CPython object
    // (`object_params`), as any caller -- CPython code included -- may pass.
    // A value takes its flattened parameters, so an unannotated (object)
    // `*args`/`**kwargs` -- which a CPython caller would bind -- stays nested.
    let non_strict = crate::desugar::dataclasses::is_non_strict();
    let parameters = &def.parameters;
    parameters
        .vararg
        .iter()
        .chain(&parameters.kwarg)
        .all(|p| p.annotation.is_some())
        && parameters
            .args
            .iter()
            .all(|p| p.default.is_none() && (non_strict || p.parameter.annotation.is_some()))
}

/// Non-strict mode: annotates `def`'s unannotated parameters and return
/// `object`, so the function value's type admits whatever its callers pass
/// and it crosses into CPython as a callable
/// (`pycc_types::foreign::is_boxable_callable`).
pub(super) fn object_params(def: &mut StmtFunctionDef) {
    if !crate::desugar::dataclasses::is_non_strict() {
        return;
    }
    let range = def.range;
    if def.returns.is_none() {
        def.returns = Some(Box::new(crate::desugar::build::load("object", range)));
    }
    for p in def.parameters.args.iter_mut() {
        if p.parameter.annotation.is_none() {
            p.parameter.annotation = Some(Box::new(crate::desugar::build::load("object", range)));
        }
    }
}

/// Whether the function `name` defined by top-level statement `def_index`
/// of `body` is used as a value only after that statement: CPython raises
/// `UnboundLocalError` (or `NameError`) for an earlier use, which would
/// otherwise bind captured variables not yet assigned. A use inside a
/// function or lambda defined earlier counts as an earlier use, since it
/// may run before the `def`.
pub(super) fn used_after_def(body: &[Stmt], name: &str, def_index: usize) -> bool {
    super::count_uses(&body[..def_index], name).other == 0
}

/// Whether every one of `captures` keeps one value from before the
/// top-level statement `def_index` of `body` (the enclosing function's
/// body, whose parameters are `params` and whose binding sites are
/// `sites`, from `super::binding_sites`) on.
pub(super) fn stable_captures(
    body: &[Stmt],
    sites: &HashMap<String, usize>,
    params: &HashSet<String>,
    def_index: usize,
    captures: &[String],
) -> bool {
    // `from m import *` may rebind any of them.
    if sites.contains_key("*") {
        return false;
    }
    captures.iter().all(|capture| {
        let bindings = sites.get(capture).copied().unwrap_or(0);
        if params.contains(capture) {
            return bindings == 0;
        }
        // The one binding is an unconditional top-level assignment before
        // the `def`, so it has run whenever the value is made.
        bindings == 1
            && body[..def_index].iter().any(|stmt| match stmt {
                Stmt::Assign(a) => {
                    a.targets.len() == 1
                        && matches!(&a.targets[0], Expr::Name(n) if n.id.as_str() == capture)
                }
                Stmt::AnnAssign(a) => {
                    a.value.is_some()
                        && matches!(a.target.as_ref(), Expr::Name(n) if n.id.as_str() == capture)
                }
                _ => false,
            })
    })
}

/// The index of the top-level statement of `body` defining `name`, if one
/// does.
pub(super) fn def_index(body: &[Stmt], name: &str) -> Option<usize> {
    body.iter()
        .position(|stmt| matches!(stmt, Stmt::FunctionDef(def) if def.name.as_str() == name))
}
