//! `match` statement checking: patterns, exhaustiveness and branch joins.

use super::*;

/// PEP 634-636 (#381, PR-21): joins N case environments from a `match`
/// statement back into `env`. If `exhaustive`, a binding present in all
/// case envs is `Definitely` (one case always runs); a binding present in
/// only some is `Maybe`. If not exhaustive, there is an implicit "no match"
/// path, so every case-only binding is `Maybe`. Pre-existing bindings are
/// preserved (first-assignment-wins, matching `join_if_branches`).
pub(crate) fn join_match_branches(
    env: &mut Environment,
    case_envs: &[Environment],
    exhaustive: bool,
) {
    let mut joined: HashMap<String, BindingState> = HashMap::new();
    let all_names: HashSet<&String> = case_envs.iter().flat_map(|ce| ce.bindings.keys()).collect();
    for name in all_names {
        let states: Vec<&BindingState> = case_envs
            .iter()
            .filter_map(|ce| ce.bindings.get(name))
            .collect();
        let ty = states[0].ty().clone();
        let all_definite = !exhaustive
            || states.len() == case_envs.len()
                && states
                    .iter()
                    .all(|s| matches!(*s, BindingState::Definitely(_)));
        if all_definite {
            joined.insert(name.clone(), BindingState::Definitely(ty));
        } else {
            joined.insert(name.clone(), BindingState::Maybe(ty));
        }
    }
    env.bindings = joined;
    // Part 2a of #1142 (#1165): see `join_if_branches` -- provenance is a
    // union over every case environment.
    for case_env in case_envs {
        env.owned_buffers
            .extend(case_env.owned_buffers.iter().cloned());
    }
    // Blocker fix (D-068 review of #780): reconcile `narrowed` the same
    // conservative way as `join_if_branches`/`join_loop_body`. `env` itself
    // (pre-match) stands in for the implicit "no case matched" path -- safe
    // to include unconditionally (exhaustive or not): it never *adds* a
    // name to the intersection, since it can only narrow the result set
    // further, so an exhaustive match that never actually needed the
    // implicit path is unaffected wherever every case agrees anyway.
    let case_narrowed_maps: Vec<&HashMap<String, Ty>> =
        case_envs.iter().map(|ce| &ce.narrowed).collect();
    env.narrowed = narrow::join_narrowed(&env.narrowed, &case_narrowed_maps);
}

/// PEP 634-636 (#381, PR-21): checks a `match` statement. The subject is
/// inferred once; each case is checked in an independent env clone (like
/// `if` arms). Pattern captures are bound before the guard and body are
/// checked. Exhaustiveness is verified (`T0030` if not exhaustive, but the
/// match is still accepted — Python allows non-exhaustive match).
pub(crate) fn check_match(
    env: &mut Environment,
    local_names: &[&str],
    subject: &HirExpr,
    cases: &[HirMatchCase],
    return_ty: Option<&Ty>,
) -> Result<(), Diagnostic> {
    let subject_ty = infer_expr_in(env, local_names, subject)?;
    // Part 2 of #1026 (#1081): `match numpy.pi:` stays refused. A
    // `HirPattern::Wildcard` returns no bindings and satisfies
    // `check_exhaustive`, so before this guard the statement type-checked
    // and reached `pycc_mir`'s `lower_match`, which has no `Ty::Object`
    // handling at all. This guard refuses the whole subject before any
    // pattern binds, so it covers `HirPattern::Capture` too: since Part 1
    // of #1333 (#1362) `check_assignment` admits an object binding in a
    // function body, so the `bindings` loop below no longer refuses one.
    foreign::reject_object_operand(&subject_ty, "matching on a CPython object")?;
    let mut case_envs = Vec::with_capacity(cases.len());
    for case in cases {
        let mut case_env = env.clone();
        let bindings = check_pattern(&case_env, local_names, &case.pattern, &subject_ty)?;
        for (name, ty) in &bindings {
            check_assignment(&mut case_env, name, ty.clone())?;
        }
        if let Some(guard) = &case.guard {
            let guard_ty = infer_expr_in(&case_env, local_names, guard)?;
            if guard_ty != Ty::Bool {
                return Err(Diagnostic::error(
                    "T0021",
                    format!("match guard must be `bool`, got `{}`", guard_ty.name()),
                    Span::new(0, 0),
                ));
            }
        }
        // D-068 re-review of #780 (third round, warning finding): route
        // each case body through `narrow::check_stmt_sequence[_in_function]`
        // instead of a raw per-statement loop, so a nested early-return
        // guard inside a `match` case narrows the rest of that same case
        // body -- the identical fast-path-bypass defect the `if`/`while`
        // fast-path helpers already had fixed for finding 2, but which
        // `check_match`'s own always-raw loop had never been routed
        // through in the first place.
        match return_ty {
            Some(rt) => narrow::check_stmt_sequence_in_function(
                &mut case_env,
                local_names,
                &case.body,
                rt.clone(),
            )?,
            None => narrow::check_stmt_sequence(&mut case_env, &case.body)?,
        }
        case_envs.push(case_env);
    }
    let exhaustive = check_exhaustive(env, &subject_ty, cases);
    join_match_branches(env, &case_envs, exhaustive);
    if !exhaustive {
        return Err(Diagnostic::error(
            "T0030",
            format!(
                "non-exhaustive `match`: not every value of `{}` is covered (add a `case _:` or \
                 cover all cases)",
                subject_ty.name()
            ),
            Span::new(0, 0),
        ));
    }
    Ok(())
}

/// PEP 634-636 (#381, PR-21): checks a pattern against a subject type,
/// returning the list of `(capture_name, type)` bindings it introduces.
pub(crate) fn check_pattern(
    env: &Environment,
    local_names: &[&str],
    pattern: &HirPattern,
    subject_ty: &Ty,
) -> Result<Vec<(String, Ty)>, Diagnostic> {
    match pattern {
        HirPattern::Wildcard => Ok(vec![]),
        HirPattern::Capture(name) => Ok(vec![(name.clone(), subject_ty.clone())]),
        HirPattern::Literal(expr) => {
            let lit_ty = infer_expr_in(env, local_names, expr)?;
            if !is_assignable(lit_ty.clone(), subject_ty.clone())
                && !is_assignable(subject_ty.clone(), lit_ty.clone())
            {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "literal pattern `{}` does not match subject type `{}`",
                        lit_ty.name(),
                        subject_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            Ok(vec![])
        }
        HirPattern::Singleton(b) => {
            if *subject_ty != Ty::Bool {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "singleton pattern `True`/`False` requires a `bool` subject, got `{}`",
                        subject_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            let _ = b;
            Ok(vec![])
        }
        HirPattern::NoneSingleton => {
            if *subject_ty != Ty::None {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "`None` pattern requires a `None` subject, got `{}`",
                        subject_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            Ok(vec![])
        }
        HirPattern::Sequence(subs) => {
            let elt_ty = check_sequence_subject(subject_ty)?;
            let mut bindings = Vec::new();
            for sub in subs {
                bindings.extend(check_pattern(env, local_names, sub, &elt_ty)?);
            }
            Ok(bindings)
        }
        HirPattern::SequenceStar(subs, rest) => {
            let elt_ty = check_sequence_subject(subject_ty)?;
            let mut bindings = Vec::new();
            for sub in subs {
                bindings.extend(check_pattern(env, local_names, sub, &elt_ty)?);
            }
            if let Some(rest) = rest {
                bindings.push((rest.clone(), Ty::List(Box::new(elt_ty))));
            }
            Ok(bindings)
        }
        HirPattern::Mapping(pairs, rest) => {
            let Ty::Dict(kv) = subject_ty else {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "mapping pattern requires a `dict` subject, got `{}`",
                        subject_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            };
            let (key_ty, val_ty) = kv.as_ref();
            let mut bindings = Vec::new();
            for (key_expr, val_pat) in pairs {
                let inferred_key = infer_expr_in(env, local_names, key_expr)?;
                if inferred_key != *key_ty {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "mapping pattern key `{}` does not match dict key type `{}`",
                            inferred_key.name(),
                            key_ty.name()
                        ),
                        Span::new(0, 0),
                    ));
                }
                bindings.extend(check_pattern(env, local_names, val_pat, val_ty)?);
            }
            if let Some(rest) = rest {
                bindings.push((rest.clone(), subject_ty.clone()));
            }
            Ok(bindings)
        }
        HirPattern::Class {
            class_name,
            positional,
            keyword,
        } => check_class_pattern(
            env,
            local_names,
            class_name,
            positional,
            keyword,
            subject_ty,
        ),
        HirPattern::Or(subs) => {
            let mut all_bindings: Option<Vec<(String, Ty)>> = None;
            for sub in subs {
                let sub_bindings = check_pattern(env, local_names, sub, subject_ty)?;
                match &all_bindings {
                    None => all_bindings = Some(sub_bindings),
                    Some(existing) => {
                        let existing_names: HashSet<&String> =
                            existing.iter().map(|(n, _)| n).collect();
                        let new_names: HashSet<&String> =
                            sub_bindings.iter().map(|(n, _)| n).collect();
                        if existing_names != new_names {
                            return Err(Diagnostic::error(
                                "T0021",
                                "or-pattern alternatives must bind the same set of names"
                                    .to_string(),
                                Span::new(0, 0),
                            ));
                        }
                    }
                }
            }
            Ok(all_bindings.unwrap_or_default())
        }
        HirPattern::As(inner, name) => {
            let mut bindings = check_pattern(env, local_names, inner, subject_ty)?;
            bindings.push((name.clone(), subject_ty.clone()));
            Ok(bindings)
        }
    }
}

/// Helper for sequence pattern checking: extracts the element type from a
/// `list` or `tuple` subject.
pub(crate) fn check_sequence_subject(subject_ty: &Ty) -> Result<Ty, Diagnostic> {
    match subject_ty {
        Ty::List(elt) => Ok((**elt).clone()),
        Ty::Tuple(elems) if !elems.is_empty() => Ok(elems[0].clone()),
        _ => Err(Diagnostic::error(
            "T0021",
            format!(
                "sequence pattern requires a `list` or `tuple` subject, got `{}`",
                subject_ty.name()
            ),
            Span::new(0, 0),
        )),
    }
}

/// Helper for class pattern checking.
pub(crate) fn check_class_pattern(
    env: &Environment,
    local_names: &[&str],
    class_name: &str,
    positional: &[HirPattern],
    keyword: &[(String, HirPattern)],
    subject_ty: &Ty,
) -> Result<Vec<(String, Ty)>, Diagnostic> {
    let class_def = env.lookup_class(class_name).ok_or_else(|| {
        Diagnostic::error(
            "T0021",
            format!("class `{class_name}` is not defined"),
            Span::new(0, 0),
        )
    })?;
    let subject_is_match = match subject_ty {
        Ty::Instance(name) => name.as_str() == class_name || class_def.mro.contains(name),
        _ => false,
    };
    if !subject_is_match {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "class pattern `{class_name}` does not match subject type `{}`",
                subject_ty.name()
            ),
            Span::new(0, 0),
        ));
    }
    let mut bindings = Vec::new();
    let init_params: Vec<(String, Ty)> = class_def
        .methods
        .iter()
        .find(|(name, _)| name == "__init__")
        .and_then(|(_, mangled)| env.lookup_function(mangled))
        .map(|(params, _)| {
            params
                .iter()
                .skip(1)
                .enumerate()
                .map(|(i, ty)| (format!("__pos_{i}"), ty.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (i, pat) in positional.iter().enumerate() {
        let param_ty = init_params
            .get(i)
            .map(|(_, ty)| ty.clone())
            .unwrap_or(Ty::Infer);
        bindings.extend(check_pattern(env, local_names, pat, &param_ty)?);
    }
    for (attr, pat) in keyword {
        // #911 (Part 1 of #885): a class-level attribute is a compile-time
        // constant with no instance storage, so `case W(MIN_WIDTH=x)` has no
        // per-instance value to match against -- every instance of `W` would
        // bind the same constant, making the sub-pattern either always or
        // never matching. Reject it rather than silently binding `Ty::Infer`
        // through the `unwrap_or` below.
        if class_def
            .class_attrs
            .iter()
            .any(|(name, _, _)| name == attr)
        {
            return Err(Diagnostic::error(
                "T0044",
                format!(
                    "`{attr}` is a class-level attribute of class `{}`, not an instance \
                     attribute -- it is a compile-time constant and cannot be matched as a \
                     class-pattern keyword sub-pattern",
                    class_def.name
                ),
                Span::new(0, 0),
            ));
        }
        let attr_ty = class_def
            .attrs
            .iter()
            .find(|(name, _)| name == attr)
            .map(|(_, ty)| ty.clone())
            .unwrap_or(Ty::Infer);
        bindings.extend(check_pattern(env, local_names, pat, &attr_ty)?);
    }
    Ok(bindings)
}

/// PEP 634-636 (#381, PR-21): returns `true` if the patterns across all
/// cases cover every value of `subject_ty`. See D-169 for the algorithm.
pub(crate) fn check_exhaustive(env: &Environment, subject_ty: &Ty, cases: &[HirMatchCase]) -> bool {
    for case in cases {
        if case.guard.is_some() {
            continue;
        }
        if is_irrefutable_pattern(&case.pattern) {
            return true;
        }
    }
    match subject_ty {
        Ty::Bool => {
            let mut has_true = false;
            let mut has_false = false;
            for case in cases {
                if case.guard.is_some() {
                    continue;
                }
                collect_bool_patterns(&case.pattern, &mut has_true, &mut has_false);
            }
            has_true && has_false
        }
        Ty::Instance(name) => {
            if let Some(class_def) = env.lookup_class(name)
                && !class_def.enum_members.is_empty()
            {
                let mut covered: HashSet<&str> = HashSet::new();
                for case in cases {
                    if case.guard.is_some() {
                        continue;
                    }
                    collect_enum_member_patterns(
                        &case.pattern,
                        name,
                        &class_def.enum_members,
                        &mut covered,
                    );
                }
                class_def
                    .enum_members
                    .iter()
                    .all(|(member, _)| covered.contains(member.as_str()))
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Returns `true` if `pattern` is irrefutable (matches every value of any
/// type): wildcard, bare capture, or an or-pattern containing an irrefutable
/// sub-pattern.
pub(crate) fn is_irrefutable_pattern(pattern: &HirPattern) -> bool {
    match pattern {
        HirPattern::Wildcard | HirPattern::Capture(_) => true,
        HirPattern::Or(subs) => subs.iter().any(is_irrefutable_pattern),
        HirPattern::As(inner, _) => is_irrefutable_pattern(inner),
        _ => false,
    }
}

/// Collects `True`/`False` singleton coverage from a pattern (recursing into
/// or-patterns).
pub(crate) fn collect_bool_patterns(
    pattern: &HirPattern,
    has_true: &mut bool,
    has_false: &mut bool,
) {
    match pattern {
        HirPattern::Singleton(true) => *has_true = true,
        HirPattern::Singleton(false) => *has_false = true,
        HirPattern::Or(subs) => {
            for sub in subs {
                collect_bool_patterns(sub, has_true, has_false);
            }
        }
        _ => {}
    }
}

/// Collects enum member coverage from class patterns like `Color.RED`.
pub(crate) fn collect_enum_member_patterns<'a>(
    pattern: &HirPattern,
    class_name: &str,
    members: &'a [(String, pycc_hir::EnumMemberValue)],
    covered: &mut HashSet<&'a str>,
) {
    match pattern {
        HirPattern::Class { class_name: cn, .. } => {
            if cn == class_name {
                for (member, _) in members {
                    covered.insert(member.as_str());
                }
            }
        }
        HirPattern::Or(subs) => {
            for sub in subs {
                collect_enum_member_patterns(sub, class_name, members, covered);
            }
        }
        // `case Color.RED:` covers that member.
        HirPattern::Literal(pycc_hir::HirExpr::AttrGet { base, attr }) => {
            if matches!(base.as_ref(), pycc_hir::HirExpr::Name(n) if n == class_name)
                && let Some((member, _)) = members.iter().find(|(m, _)| m == attr)
            {
                covered.insert(member.as_str());
            }
        }
        _ => {}
    }
}
