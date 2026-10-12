//! Unannotated `__init__` parameters of a native program, from its
//! constructor calls.
//!
//! In a native build `pycc_hir` lowers an unannotated `__init__` parameter
//! as `Ty::Infer` (an `--ext` module's class is constructed from Python
//! with anything, so there it keeps its annotation requirement), and an
//! attribute it seeds (`self.n = n`) as a provisional slot. The parameter
//! has to be typed before the class phase types those slots, so it is
//! typed here, ahead of everything else, from the constructor calls
//! `C(args)` of the program: when every call the checker can type in its
//! own scope passes one concrete type, the parameter takes it. The checker
//! that follows still checks every construction against it.
//!
//! A parameter no call types, or one the calls disagree on, is the `T0001`
//! a missing annotation always was; in non-strict mode it is the CPython
//! object instead, as an unannotated parameter there always was.

use super::call_site::for_each_call;
use super::contains_infer;
use crate::infer_expr_in;
use crate::module::{DiagnosticKey, KeyedDiagnostics};
use pycc_hir::{HirItem, HirModule, Ty};
use std::collections::HashMap;

/// `hir` with every unannotated `__init__` parameter typed; `Ok(None)` when
/// there is none.
pub(crate) fn resolve_init_params(hir: &HirModule) -> Result<Option<HirModule>, KeyedDiagnostics> {
    // Mangled `__init__` name -> its class, for every class constructed by
    // name whose `__init__` has an unannotated parameter.
    let inits: HashMap<&str, &str> = hir
        .class_defs
        .iter()
        .filter(|(_, class)| class.type_param.is_none() && !class.is_protocol)
        .filter_map(|(class_name, class)| {
            let (_, mangled) = class.methods.iter().find(|(name, _)| name == "__init__")?;
            Some((mangled.as_str(), class_name.as_str()))
        })
        .filter(|(mangled, _)| {
            hir.items.iter().any(|item| {
                matches!(item, HirItem::Function { name, params, .. }
                    if name == mangled && params.iter().any(|(_, ty)| *ty == Ty::Infer))
            })
        })
        .collect();
    if inits.is_empty() {
        return Ok(None);
    }
    let constructors: HashMap<&str, &str> = inits.iter().map(|(m, c)| (*c, *m)).collect();
    // (mangled `__init__`, parameter index) -> `Some(ty)` while every
    // typable argument agrees on `ty`.
    let mut observed: HashMap<(String, usize), Option<Ty>> = HashMap::new();
    for_each_call(hir, &mut |env, names, callee, args| {
        let Some(mangled) = constructors.get(callee) else {
            return;
        };
        for (index, arg) in args.iter().enumerate() {
            // The receiver is parameter 0.
            let key = (mangled.to_string(), index + 1);
            let Ok(ty) = infer_expr_in(env, names, arg) else {
                observed.insert(key, None);
                continue;
            };
            let entry = observed.entry(key).or_insert(Some(ty.clone()));
            if contains_infer(&ty) || entry.as_ref() != Some(&ty) {
                *entry = None;
            }
        }
    });
    let mut resolved = hir.clone();
    let mut missing: KeyedDiagnostics = Vec::new();
    for (index, item) in resolved.items.iter_mut().enumerate() {
        let HirItem::Function { name, params, .. } = item else {
            continue;
        };
        if !inits.contains_key(name.as_str()) {
            continue;
        }
        for (position, (param, ty)) in params.iter_mut().enumerate() {
            if *ty != Ty::Infer {
                continue;
            }
            match observed.get(&(name.clone(), position)) {
                Some(Some(found)) => *ty = found.clone(),
                _ if crate::non_strict::is_non_strict() => *ty = Ty::Object,
                _ => missing.push((
                    DiagnosticKey::Function(index),
                    crate::locate::place_item(
                        crate::constraints::parameter_needs_annotation("__init__", param),
                        index,
                    ),
                )),
            }
        }
    }
    if !missing.is_empty() {
        return Err(missing);
    }
    Ok(Some(resolved))
}
