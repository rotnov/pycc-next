//! Lifting one function's nested functions (the module doc of [`super`]).

use super::*;

/// Lifts the nested functions of `def` (whose `__qualname__` is `owner`),
/// innermost first, appending each lifted `def` to `out`.
pub(super) fn lift_function(def: &mut StmtFunctionDef, owner: &str, fresh: &Fresh, out: &mut Vec<Stmt>) {
    super::conditional::split_conditional_defs(def);
    let mut body: Vec<Stmt> = std::mem::take(&mut def.body).into_iter().collect();
    lambdas_to_defs(&mut body);
    // Innermost first: each nested function's own nested functions.
    let mut nested: Vec<*mut StmtFunctionDef> = Vec::new();
    nested_defs_mut(&mut body, &mut nested);
    for ptr in &nested {
        // SAFETY: the pointers address distinct `def`s inside `body`, which
        // is not otherwise borrowed while they are used.
        let inner = unsafe { &mut **ptr };
        let inner_owner = format!("{owner}.<locals>.{}", inner.name.as_str());
        lift_function(inner, &inner_owner, fresh, out);
    }
    let candidates: Vec<&StmtFunctionDef> = nested.iter().map(|p| unsafe { &**p }).collect();
    let outer_names = {
        let mut probe = def.clone();
        probe.body = body.clone().into();
        scope_names(&probe)
    };
    // A function used as a value whose captures are not stable is dropped
    // and the choice made again (`values`).
    let params: HashSet<String> = def
        .parameters
        .iter()
        .map(|p| p.name().to_string())
        .collect();
    let sites = binding_sites(&body);
    let mut excluded: HashSet<String> = HashSet::new();
    let (chosen, chosen_names, cells, captures) = loop {
        // Choose the liftable ones.
        let mut chosen: Vec<&StmtFunctionDef> = Vec::new();
        for candidate in &candidates {
            let name = candidate.name.as_str();
            let uses = count_uses(&body, name);
            if !liftable_shape(candidate)
                || def
                    .parameters
                    .iter()
                    .any(|param| param.name().as_str() == name)
                || uses.bindings != 1
                || (uses.other != 0
                    && (excluded.contains(name)
                        || !values::value_shape(candidate)
                        || values::def_index(&body, name).is_none()))
                || outer_names.globals.contains(name)
                || outer_names.nonlocals.contains(name)
                || candidates
                    .iter()
                    .filter(|c| c.name.as_str() == name)
                    .count()
                    != 1
            {
                continue;
            }
            chosen.push(candidate);
        }
        if chosen.is_empty() {
            def.body = body.into();
            return;
        }
        let chosen_names: HashSet<String> = chosen.iter().map(|d| d.name.to_string()).collect();
        // Cells: enclosing variables a chosen function declares `nonlocal`.
        let mut cells: HashSet<String> = HashSet::new();
        let mut infos: HashMap<String, ScopeNames> = HashMap::new();
        for candidate in &chosen {
            let names = scope_names(candidate);
            for nonlocal in &names.nonlocals {
                cells.insert(nonlocal.clone());
            }
            infos.insert(candidate.name.to_string(), names);
        }
        let celled_ok = cells.iter().all(|cell| {
            outer_names.bound.contains(cell)
                && only_plain_assignments(&body, cell)
                && !chosen_names.contains(cell)
        });
        if !celled_ok {
            def.body = body.into();
            return;
        }
        // Captures: what each reads (or writes nonlocally) of the enclosing
        // scope, closed over the calls between chosen functions.
        let enclosing: HashSet<String> = outer_names.bound.clone();
        let mut captures: HashMap<String, BTreeSet<String>> = HashMap::new();
        for (name, names) in &infos {
            let own: BTreeSet<String> = names
                .free
                .iter()
                .chain(names.nonlocals.iter())
                .filter(|n| enclosing.contains(*n) && !chosen_names.contains(*n))
                .cloned()
                .collect();
            captures.insert(name.clone(), own);
        }
        loop {
            let mut changed = false;
            for (name, names) in &infos {
                let called: Vec<String> = names
                    .free
                    .iter()
                    .filter(|n| chosen_names.contains(*n) && *n != name)
                    .cloned()
                    .collect();
                for callee in called {
                    let extra = captures[&callee].clone();
                    let mine = captures.get_mut(name).expect("every chosen function");
                    for capture in extra {
                        changed |= mine.insert(capture);
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let unstable: Vec<String> = chosen
            .iter()
            .map(|c| c.name.to_string())
            .filter(|name| {
                count_uses(&body, name).other != 0
                    && !values::def_index(&body, name).is_some_and(|index| {
                        let own: Vec<String> = captures[name].iter().cloned().collect();
                        values::used_after_def(&body, name, index)
                            && values::stable_captures(&body, &sites, &params, index, &own)
                    })
            })
            .collect();
        if unstable.is_empty() {
            break (chosen, chosen_names, cells, captures);
        }
        excluded.extend(unstable);
    };
    let lifts: HashMap<String, Lift> = chosen
        .iter()
        .map(|candidate| {
            let name = candidate.name.to_string();
            let lifted = qualname::lifted_name(
                &format!("{owner}.<locals>.{name}"),
                fresh.name().trim_start_matches("_pycc_"),
            );
            let lift = Lift {
                lifted,
                captures: captures[&name].iter().cloned().collect(),
            };
            (name, lift)
        })
        .collect();
    let rewrite = Rewrite {
        lifts: &lifts,
        cells: &cells,
        in_lifted: true,
    };
    // The lifted functions, in source order.
    for candidate in &chosen {
        let lift = &lifts[candidate.name.as_str()];
        let mut lifted_def: StmtFunctionDef = (*candidate).clone();
        if count_uses(&body, candidate.name.as_str()).other != 0 {
            values::object_params(&mut lifted_def);
        }
        let range = lifted_def.range;
        let mut params: Vec<ParameterWithDefault> = lift
            .captures
            .iter()
            .map(|capture| {
                let name = if cells.contains(capture) {
                    cell_name(capture)
                } else {
                    capture.clone()
                };
                param(&name, range)
            })
            .collect();
        // The captures come first: ahead of any positional-only parameter
        // too (the `*args`/keyword-only/`**kwargs` parameters stay where
        // they are, `signatures` binds calls of the lifted function).
        if lifted_def.parameters.posonlyargs.is_empty() {
            params.extend(lifted_def.parameters.args.iter().cloned());
            lifted_def.parameters.args = params.into();
        } else {
            params.extend(lifted_def.parameters.posonlyargs.iter().cloned());
            lifted_def.parameters.posonlyargs = params.into();
        }
        lifted_def.name = Identifier::new(lift.lifted.as_str(), range);
        if lifted_def.returns.is_none() {
            lifted_def.returns = Some(Box::new(load(INFER_MARKER, range)));
        }
        let mut lifted_body: Vec<Stmt> = std::mem::take(&mut lifted_def.body).into_iter().collect();
        for stmt in lifted_body.iter_mut() {
            rewrite.visit_stmt(stmt);
        }
        lifted_def.body = lifted_body.into();
        out.push(Stmt::FunctionDef(lifted_def));
    }
    // The enclosing function: drop the lifted defs, then rewrite calls and
    // cells; a celled parameter starts in a cell.
    let lifted_names: HashSet<String> = chosen_names;
    remove_defs(&mut body, &lifted_names);
    let rewrite = Rewrite {
        in_lifted: false,
        ..rewrite
    };
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
    let range = def.range;
    let mut prologue: Vec<Stmt> = def
        .parameters
        .iter()
        .filter(|p| cells.contains(p.name().as_str()))
        .map(|p| assign_cell(p.name().as_str(), load(p.name().as_str(), range), range))
        .collect();
    prologue.extend(body);
    def.body = prologue.into();
}
