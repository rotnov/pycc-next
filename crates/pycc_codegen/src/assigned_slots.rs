//! Whole-program proof that an instance attribute slot is always assigned
//! when compiled code reads it, so the read needs no "assigned" flag test
//! and a later store needs no flag store (#1388's flag, D-154's layout).
//!
//! A slot is unassigned only between `pycc_rt_instance_new` and the first
//! store into it: a native build has no `del obj.attr` for a class instance.
//! Every instance of a class of this program is made by a
//! `MirExpr::Instantiate`, which allocates it and calls the class's `ctor`
//! with it as `self` (enum singletons, the other allocation, are assigned
//! before any code runs and are never this analysis's subject).
//!
//! A constructor's *assigning prefix* is the leading run of its statements
//! that are either `self.a = value` or a local assignment / expression
//! statement, where nothing in the run mentions `self` (other than as the
//! base of those stores). Until the prefix ends the instance cannot have
//! escaped: no code outside the constructor holds it, and the constructor
//! itself has not read it. If any statement of the prefix raises, the
//! constructor fails and the instance is never seen. So every attribute the
//! prefix stores is assigned before anything can observe the instance.
//!
//! An attribute of class `C` is *always assigned* when, for every class `D`
//! of the program whose MRO contains `C` (`C` itself included) and every
//! constructor some `Instantiate` uses for `D`, that constructor's
//! assigning prefix stores it. A class no `Instantiate` makes contributes no
//! constructor (it has no instances). A class whose MRO reaches outside this
//! program (an exception class), a generic class this program only knows by
//! its monomorphised name, or a constructor defined more than once proves
//! nothing.
//!
//! An `--ext` build proves nothing at all: its host can make an instance
//! without running `__init__` and can delete an attribute.

use pycc_hir::HirClassDef;
use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt, Ty};
use std::collections::{HashMap, HashSet};

/// The proof's result: the `(class, slot)` pairs always assigned, and the
/// constructors (which must keep storing the flag their prefix sets).
#[derive(Default)]
pub(crate) struct AssignedSlots {
    always: HashSet<(String, usize)>,
    ctors: HashSet<String>,
}

impl AssignedSlots {
    /// Runs the analysis over `mir` (`ext`: an `--ext` build, which proves
    /// nothing).
    pub(crate) fn analyze(mir: &MirModule, ext: bool) -> Self {
        if ext {
            return Self::default();
        }
        let classes: HashMap<&str, &HirClassDef> = mir
            .class_defs
            .iter()
            .map(|(name, def)| (name.as_str(), def))
            .collect();
        let layouts: HashMap<&str, Vec<String>> = classes
            .iter()
            .filter_map(|(name, def)| Some((*name, layout(def, &classes)?)))
            .collect();
        let mut functions: HashMap<&str, Vec<(&[(String, Ty)], &[MirStmt])>> = HashMap::new();
        for item in &mir.items {
            if let MirItem::Function {
                name, params, body, ..
            } = item
            {
                functions
                    .entry(name.as_str())
                    .or_default()
                    .push((params.as_slice(), body.as_slice()));
            }
        }
        let mut instantiations: Vec<(String, String)> = Vec::new();
        for item in &mir.items {
            let stmts: Vec<&MirStmt> = match item {
                MirItem::Function { body, .. } => body.iter().collect(),
                MirItem::TopLevelStmt(stmt) => vec![stmt],
                MirItem::ForeignImport { .. } => Vec::new(),
            };
            for stmt in stmts {
                collect_instantiations_stmt(stmt, &mut instantiations);
            }
        }
        // The attribute names each constructor's prefix assigns.
        let mut prefix_names: HashMap<&str, Option<HashSet<String>>> = HashMap::new();
        for (_, ctor) in &instantiations {
            prefix_names.entry(ctor.as_str()).or_insert_with(|| {
                let [(params, body)] = functions.get(ctor.as_str())?.as_slice() else {
                    return None;
                };
                let (self_name, self_ty) = params.first()?;
                let Ty::Instance(self_class) = self_ty else {
                    return None;
                };
                let self_layout = layouts.get(self_class.as_str())?;
                Some(assigning_prefix(self_name, body, self_layout))
            });
        }
        let mut always = HashSet::new();
        for class in classes.keys() {
            let Some(class_layout) = layouts.get(class) else {
                continue;
            };
            // Every constructor of every class whose MRO contains `class`.
            let mut ctor_sets: Vec<&Option<HashSet<String>>> = Vec::new();
            let mut known = true;
            for (instance_class, ctor) in &instantiations {
                let Some(instance_def) = classes.get(instance_class.as_str()) else {
                    known = false;
                    break;
                };
                if instance_def.mro.iter().any(|entry| entry == class) {
                    ctor_sets.push(&prefix_names[ctor.as_str()]);
                }
            }
            if !known {
                continue;
            }
            for (slot, attr) in class_layout.iter().enumerate() {
                if ctor_sets
                    .iter()
                    .all(|names| names.as_ref().is_some_and(|names| names.contains(attr)))
                {
                    always.insert(((*class).to_string(), slot));
                }
            }
        }
        let ctors = instantiations.into_iter().map(|(_, ctor)| ctor).collect();
        Self { always, ctors }
    }

    /// Whether slot `slot` of an instance typed `base_ty` is always assigned.
    pub(crate) fn is_always_assigned(&self, base_ty: &Ty, slot: usize) -> bool {
        match base_ty {
            Ty::Instance(class) => self.always.contains(&(class.to_string(), slot)),
            _ => false,
        }
    }

    /// Whether `function` (a MIR function name) is some `Instantiate`'s
    /// constructor, or any `__init__`.
    pub(crate) fn is_ctor(&self, function: &str) -> bool {
        self.ctors.contains(function) || function.ends_with("__init__")
    }
}

/// The class's flat slot names, when every class of its MRO is this
/// program's.
fn layout(def: &HirClassDef, classes: &HashMap<&str, &HirClassDef>) -> Option<Vec<String>> {
    let mro: Vec<&HirClassDef> = def
        .mro
        .iter()
        .map(|entry| classes.get(entry.as_str()).copied())
        .collect::<Option<_>>()?;
    Some(
        pycc_hir::flat_attr_layout(&mro)
            .into_iter()
            .map(|(name, _)| name)
            .collect(),
    )
}

fn collect_instantiations_stmt(stmt: &MirStmt, out: &mut Vec<(String, String)>) {
    for expr in stmt.exprs() {
        collect_instantiations_expr(expr, out);
    }
    for body in stmt.bodies() {
        for stmt in body {
            collect_instantiations_stmt(stmt, out);
        }
    }
}

fn collect_instantiations_expr(expr: &MirExpr, out: &mut Vec<(String, String)>) {
    if let MirExpr::Instantiate(inst) = expr {
        let class = match &inst.ty {
            Ty::Instance(class) => class.to_string(),
            // A class this analysis cannot name proves nothing for anyone:
            // an empty class name matches no class definition.
            _ => String::new(),
        };
        out.push((class, inst.ctor.clone()));
    }
    for child in expr.children() {
        collect_instantiations_expr(child, out);
    }
}

/// Whether `expr` reads or (through a walrus) rebinds the name `self_name`
/// anywhere.
fn mentions(expr: &MirExpr, self_name: &str) -> bool {
    matches!(expr, MirExpr::Name { name, .. } | MirExpr::NamedExpr { name, .. } if name == self_name)
        || expr.children().into_iter().any(|child| mentions(child, self_name))
}

/// The attributes `body`'s assigning prefix stores into `self_name` (see the
/// module doc), named through the constructor's own `self` layout.
fn assigning_prefix(self_name: &str, body: &[MirStmt], self_layout: &[String]) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in body {
        match stmt {
            MirStmt::AttrSet { base, slot, value }
                if matches!(base, MirExpr::Name { name, .. } if name == self_name)
                    && !mentions(value, self_name) =>
            {
                match self_layout.get(*slot) {
                    Some(attr) => {
                        names.insert(attr.clone());
                    }
                    None => break,
                }
            }
            MirStmt::Assign { target, value } if target != self_name && !mentions(value, self_name) => {}
            MirStmt::ExprStmt(value) if !mentions(value, self_name) => {}
            MirStmt::NoOp => {}
            _ => break,
        }
    }
    names
}

