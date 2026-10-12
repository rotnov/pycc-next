//! Lowers typed numpy arrays out of a linked program (`super`'s module doc).
//!
//! Runs once, at the end of `crate::finalize`, before any type checking.
//! After it no `Ty::NdArray` is left anywhere: a typed array is an
//! `object` from here on, and every operation the compiler runs natively is
//! a `HirExpr::Nd` node whose own fields carry the dtype and dimension
//! count. Everything else done with an array (`print(a)`, `a.sum()`,
//! `np.dot(a, b)`) is the ordinary object operation it would be on any
//! CPython value.
//!
//! **Which names are typed arrays.** A name is a typed array in a scope
//! (a function body, or the module body) when an annotation says so --
//! a parameter or an annotated assignment -- or when every binding of it
//! in that scope is a value of one statically known array type:
//!
//! * a numpy creation call whose dtype and shape are evident from the
//!   call: `np.zeros(n)`, `np.zeros((n, m), dtype=np.int64)`,
//!   `np.ones`/`np.empty`/`np.full`, `np.array([[1.0, 2.0], [3.0, 4.0]])`,
//!   `np.arange(n)`, `np.zeros_like(a)`, `a.copy()`;
//! * another typed array name, a call of a function (or method) whose
//!   return annotation is a typed array, an attribute whose class declares
//!   it a typed array, or a row `a[i]` of a two-dimensional one.
//!
//! **Checks.** Each binding of a typed array name whose value is not
//! already known to be one of that type (a creation call, an annotated
//! value of unknown origin, a parameter) is followed by
//! `NdOp::Check`, which raises `TypeError` when the object is not a
//! `numpy.ndarray` with the static dtype and dimension count. A
//! parameter is checked at function entry, so a caller never checks.
//!
//! **Operations.** `a[i]` / `a[i, j]` / `a[i][j]` reads and writes,
//! `a.shape`, `a.shape[k]`, `len(a)`, `a.size`, `a.ndim` and `for x in a`
//! over a one-dimensional array become `HirExpr::Nd` nodes. A row `a[i]`
//! of a two-dimensional array that is not indexed again stays an object
//! subscript (numpy builds the row view), as does any slice.

use super::{NdDtype, NdExpr, NdOp, NdTy, expr_children, expr_children_mut};
use crate::{HirExpr, HirItem, HirModule, HirStmt, ImportBinding, Ty};
use std::collections::{HashMap, HashSet};

mod bindings;
mod rewrite;
mod values;
use bindings::*;
use rewrite::*;
use values::*;

/// Module-wide facts.
struct Ctx {
    /// Names bound to the `numpy` module itself (`import numpy as np`).
    numpy: HashSet<String>,
    /// Functions (methods by their mangled name) returning a typed array.
    fn_ret: HashMap<String, NdTy>,
    /// Functions returning `int`.
    fn_int: HashSet<String>,
    /// Every function the program defines (a `def len` shadows the builtin).
    functions: HashSet<String>,
    /// `(class, attribute)` declared a typed array.
    class_attrs: HashMap<(String, String), NdTy>,
    /// `(class, method)` -> mangled function name.
    methods: HashMap<(String, String), String>,
    /// The module scope's facts, consulted for names a function does not
    /// bind itself.
    module: Facts,
    fresh: usize,
}

/// What a scope knows about its names.
#[derive(Default, Clone)]
struct Facts {
    nd: HashMap<String, NdTy>,
    ints: HashSet<String>,
    floats: HashSet<String>,
    tuples: HashMap<String, usize>,
    instances: HashMap<String, String>,
    /// Every name the scope binds.
    locals: HashSet<String>,
    /// The typed array locals of a function every binding of which the
    /// rewrite follows with a `Check` (`=`, an annotated `=`, a parameter,
    /// `for` over a typed array): their operations are `NdExpr::cached`.
    /// A name also bound some other way (`for x in objs`, `except ... as`,
    /// a `match` capture, `:=`, `del`) is checked at every use instead.
    cache_safe: HashSet<String>,
}

/// One binding of a name, as the scope analysis sees it.
enum Binding<'a> {
    Value(&'a HirExpr),
    Annotated(&'a Ty),
    Int,
    /// The variable of `for var in list`: a row when `list` is a
    /// two-dimensional typed array.
    Row(&'a str),
    Other,
}

fn erase(ty: &mut Ty) {
    match ty {
        Ty::NdArray(_) => *ty = Ty::Object,
        Ty::List(inner)
        | Ty::Set(inner)
        | Ty::FrozenSet(inner)
        | Ty::Optional(inner)
        | Ty::Generator(inner) => {
            erase(inner);
        }
        Ty::Dict(pair) => {
            erase(&mut pair.0);
            erase(&mut pair.1);
        }
        Ty::Tuple(elements) => elements.iter_mut().for_each(erase),
        Ty::Callable(sig) => {
            sig.0.iter_mut().for_each(erase);
            erase(&mut sig.1);
        }
        _ => {}
    }
}

fn mentions_nd(ty: &Ty) -> bool {
    let mut copy = ty.clone();
    erase(&mut copy);
    copy != *ty
}

/// Whether the module uses typed arrays at all (a cheap early exit).
fn uses_typed_arrays(hir: &HirModule) -> bool {
    let numpy = hir.imports.iter().any(is_numpy_import);
    numpy
        || hir
            .class_defs
            .iter()
            .any(|(_, c)| c.attrs.iter().any(|(_, t)| mentions_nd(t)))
        || hir.items.iter().any(|item| match item {
            HirItem::Function {
                params, return_ty, ..
            } => params.iter().any(|(_, t)| mentions_nd(t)) || mentions_nd(return_ty),
            HirItem::TopLevelStmt(_) => false,
        })
}

fn is_numpy_import(binding: &ImportBinding) -> bool {
    matches!(
        binding,
        ImportBinding::Foreign { module_path, from: None, local_name, .. }
            if module_path == "numpy" && !local_name.contains('.')
    )
}

/// Lowers every typed array of `hir` (see the module doc).
pub fn lower_typed_arrays(hir: &mut HirModule) {
    if !uses_typed_arrays(hir) {
        return;
    }
    let mut ctx = Ctx {
        numpy: hir
            .imports
            .iter()
            .filter(|b| is_numpy_import(b))
            .filter_map(|b| match b {
                ImportBinding::Foreign { local_name, .. } => Some(local_name.clone()),
                _ => None,
            })
            .collect(),
        fn_ret: HashMap::new(),
        fn_int: HashSet::new(),
        functions: HashSet::new(),
        class_attrs: HashMap::new(),
        methods: HashMap::new(),
        module: Facts::default(),
        fresh: 0,
    };
    for item in &hir.items {
        if let HirItem::Function {
            name, return_ty, ..
        } = item
        {
            ctx.functions.insert(name.clone());
            match return_ty {
                Ty::NdArray(nd) => {
                    ctx.fn_ret.insert(name.clone(), *nd);
                }
                Ty::Int => {
                    ctx.fn_int.insert(name.clone());
                }
                _ => {}
            }
        }
    }
    for (class_name, class) in &mut hir.class_defs {
        for (attr, ty) in &mut class.attrs {
            if let Ty::NdArray(nd) = ty {
                ctx.class_attrs
                    .insert((class_name.clone(), attr.clone()), *nd);
            }
            erase(ty);
        }
        for (method, mangled) in &class.methods {
            ctx.methods
                .insert((class_name.clone(), method.clone()), mangled.clone());
        }
    }
    // A subclass's instances carry its bases' array attributes too.
    for (class_name, class) in &hir.class_defs {
        for ancestor in &class.mro {
            let inherited: Vec<(String, NdTy)> = ctx
                .class_attrs
                .iter()
                .filter(|((owner, _), _)| owner == ancestor)
                .map(|((_, attr), nd)| (attr.clone(), *nd))
                .collect();
            for (attr, nd) in inherited {
                ctx.class_attrs.entry((class_name.clone(), attr)).or_insert(nd);
            }
        }
    }
    super::record_typed_slots(&ctx.class_attrs);
    // The module scope first: functions read its facts.
    let top: Vec<&HirStmt> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::TopLevelStmt(stmt) => Some(stmt),
            HirItem::Function { .. } => None,
        })
        .collect();
    let module_facts = analyze(&ctx, &[], top.iter().copied(), true);
    ctx.module = module_facts.clone();
    for item in &mut hir.items {
        match item {
            HirItem::Function {
                name,
                params,
                return_ty,
                body,
            } => {
                let facts = analyze(&ctx, params, body.iter(), false);
                let ret = match return_ty {
                    Ty::NdArray(nd) => Some(*nd),
                    _ => None,
                };
                let mut lowered = Vec::with_capacity(body.len() + params.len());
                for (param, ty) in params.iter_mut() {
                    if let Ty::NdArray(nd) = ty {
                        let cached = facts.cache_safe.contains(param.as_str());
                        lowered.push(check_stmt(HirExpr::Name(param.clone()), *nd, cached));
                    }
                    erase(ty);
                }
                erase(return_ty);
                let _ = name;
                let mut scope = Scope {
                    ctx: &mut ctx,
                    facts: &facts,
                    ret,
                };
                lowered.extend(scope.rewrite_body(std::mem::take(body)));
                *body = lowered;
            }
            HirItem::TopLevelStmt(_) => {}
        }
    }
    // Top-level statements are separate items; rewrite each in place.
    let mut items = Vec::with_capacity(hir.items.len());
    for item in std::mem::take(&mut hir.items) {
        match item {
            HirItem::TopLevelStmt(stmt) => {
                let mut scope = Scope {
                    ctx: &mut ctx,
                    facts: &module_facts,
                    ret: None,
                };
                for stmt in scope.rewrite_body(vec![stmt]) {
                    items.push(HirItem::TopLevelStmt(stmt));
                }
            }
            other => items.push(other),
        }
    }
    hir.items = items;
    for (_, ty) in &mut hir.type_aliases {
        erase(ty);
    }
}
