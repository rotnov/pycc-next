//! PEP 634-636 structural pattern matching lowering (#546): `lower_match` and
//! the pattern-condition helpers that desugar a `match` into nested `if`s.

use super::{
    HirClassDef, MATCH_SUBJECT_COUNTER, MirExpr, MirStmt, bind_variable, lower_expr,
    lower_scoped_body, mro_attrs,
};
use pycc_hir::{CmpOpKind, HirExpr, HirMatchCase, HirPattern, Ty};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

/// PEP 634-636 (#381, PR-21): Lowers a `match` statement into nested
/// `MirStmt::If` chains. The subject is evaluated once and stored in a
/// synthesized temporary (`__match_subj_N`); each case becomes an `if`
/// branch whose test is the pattern-match condition, whose body is the
/// case body (preceded by binding assignments), and whose `orelse` is
/// the next case (or `NoOp` for the final arm). Guards are handled via
/// a nested `if` inside the matched arm's body, since MIR has no `and`.
pub(super) fn lower_match(
    subject: &HirExpr,
    cases: &[HirMatchCase],
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirStmt {
    let subj_expr = lower_expr(subject, scopes, classes, current_class);
    let subj_ty = subj_expr.ty();
    let subj_var = format!(
        "__match_subj_{}",
        MATCH_SUBJECT_COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    bind_variable(scopes, subj_var.clone(), subj_ty.clone());
    let assign = MirStmt::Assign {
        target: subj_var.clone(),
        value: subj_expr,
    };
    let chain = lower_match_chain(&subj_var, &subj_ty, cases, scopes, classes, current_class);
    MirStmt::Seq(vec![assign, chain])
}

/// Builds the nested `if` chain for match cases. Each case's pattern
/// produces a list of alternative condition-lists (any alternative's
/// conditions all being true is sufficient); these are nested as `if`
/// statements, with the innermost body containing the bindings and case
/// body (or a guard `if` if present).
fn lower_match_chain(
    subj_var: &str,
    subj_ty: &Ty,
    cases: &[HirMatchCase],
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirStmt {
    if cases.is_empty() {
        // `pycc_types::check` rejects every non-exhaustive match before MIR
        // lowering (T0030), including guarded-only coverage. Reaching the
        // end of the validated case chain is therefore statically
        // impossible and must remain terminal in MIR.
        return MirStmt::Unreachable;
    }
    let case = &cases[0];
    let rest = &cases[1..];
    let subj_ref = MirExpr::Name {
        name: subj_var.to_string(),
        ty: subj_ty.clone(),
    };
    let (alternatives, bindings) =
        lower_pattern_conds(&subj_ref, &case.pattern, scopes, classes, current_class);
    for (name, val) in &bindings {
        let ty = val.ty();
        bind_variable(scopes, name.clone(), ty);
        // D-068 re-review of #780 (fifth round): a pattern-capture binding
        // (`case x:`, or a capture nested in `Sequence`/`SequenceStar`/
        // `Mapping`/`Class`/`Or`/`As`) rebinds `name` exactly like `Assign`/
        // `AnnAssign`/the `Try`-handler `as` binding do -- each of those
        // pairs its own `bind_variable`/`bind` call with `kill_narrowing`
        // (see `stmt.rs`'s `Assign`/`AnnAssign` arms, `stmt::try_stmt`, and `expr.rs`'s
        // `pre_bind_named_expr_targets`), but this call site never did.
        // Without this, a name narrowed by an enclosing `if name is not
        // None:` kept its stale `$narrowed:{name}` sentinel after a `match`
        // case captured the same name, so a later read of `name` -- even
        // outside the `match` entirely, since this binding is applied
        // directly to `scopes` rather than inside `case.body`'s isolated
        // snapshot -- would still wrongly lower to `MirExpr::OptionalUnwrap`
        // against the pre-match narrowed type instead of the pattern
        // capture's real type.
        super::kill_narrowing(scopes, name);
    }
    let binding_stmts: Vec<MirStmt> = bindings
        .iter()
        .map(|(name, value)| MirStmt::Assign {
            target: name.clone(),
            value: value.clone(),
        })
        .collect();
    // D-068 review of #780: `match` case bodies are not this fix's scope --
    // each case is already isolated from its siblings by
    // `lower_scoped_body`'s own snapshot/restore, and the checker's own
    // (now narrowing-aware) `join_match_branches` gate means no HIR this
    // crate lowers can rely on a narrowing fact this ending state would
    // have supplied -- see `lower_scoped_body`'s doc comment. The ending
    // narrowed state is intentionally discarded here.
    let (case_body, _end_narrowed) =
        lower_scoped_body(&case.body, scopes, classes, current_class, None);
    let else_chain = lower_match_chain(subj_var, subj_ty, rest, scopes, classes, current_class);
    let inner_body = if let Some(guard) = &case.guard {
        let guard_cond = lower_expr(guard, scopes, classes, current_class);
        let mut body = binding_stmts;
        body.push(MirStmt::If {
            test: guard_cond,
            body: case_body,
            orelse: vec![else_chain.clone()],
        });
        body
    } else {
        let mut body = binding_stmts;
        body.extend(case_body);
        body
    };
    nest_match_alternatives(&alternatives, inner_body, else_chain)
}

/// Nests alternative condition-lists into a chain of `if` statements.
/// Each alternative is a conjunction (all conditions must be true); the
/// alternatives are combined as a disjunction (any one matching is
/// sufficient). The innermost `then` body is `inner_body`; each
/// alternative's `else` falls through to the next alternative, and the
/// last alternative's `else` falls through to `else_chain`.
pub(super) fn nest_match_alternatives(
    alternatives: &[Vec<MirExpr>],
    inner_body: Vec<MirStmt>,
    else_chain: MirStmt,
) -> MirStmt {
    if alternatives.is_empty() {
        return MirStmt::Seq(inner_body);
    }
    let (first, rest) = alternatives.split_first().unwrap();
    let next_else = if rest.is_empty() {
        else_chain.clone()
    } else {
        nest_match_alternatives(rest, inner_body.clone(), else_chain.clone())
    };
    nest_match_conds(first, inner_body, next_else)
}

/// Nests a single alternative's conditions into a chain of `if`
/// statements. The innermost `then` body is `inner_body`; every `else`
/// falls through to `else_chain`.
fn nest_match_conds(conds: &[MirExpr], inner_body: Vec<MirStmt>, else_chain: MirStmt) -> MirStmt {
    if conds.is_empty() {
        return MirStmt::Seq(inner_body);
    }
    let (first, rest) = conds.split_first().unwrap();
    MirStmt::If {
        test: first.clone(),
        body: vec![nest_match_conds(
            rest,
            inner_body.clone(),
            else_chain.clone(),
        )],
        orelse: vec![else_chain],
    }
}

/// Lowers a pattern into a list of alternative condition-lists (each
/// inner list is a conjunction; the outer list is a disjunction) and a
/// list of binding assignments. For irrefutable patterns (wildcard,
/// capture), the alternatives list contains a single empty inner list
/// (always matches). For Or-patterns, each sub-pattern's alternatives
/// are flattened into the outer list.
fn lower_pattern_conds(
    subj: &MirExpr,
    pattern: &HirPattern,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> (Vec<Vec<MirExpr>>, Vec<(String, MirExpr)>) {
    match pattern {
        HirPattern::Wildcard => (vec![vec![]], vec![]),
        HirPattern::Capture(name) => (vec![vec![]], vec![(name.clone(), subj.clone())]),
        HirPattern::Literal(lit) => {
            let lowered = lower_expr(lit, scopes, classes, current_class);
            (
                vec![vec![MirExpr::Compare {
                    op: CmpOpKind::Eq,
                    left: Box::new(subj.clone()),
                    right: Box::new(lowered),
                    ty: Ty::Bool,
                }]],
                vec![],
            )
        }
        HirPattern::Singleton(b) => (
            vec![vec![MirExpr::Compare {
                op: CmpOpKind::Eq,
                left: Box::new(subj.clone()),
                right: Box::new(MirExpr::BoolLiteral(*b)),
                ty: Ty::Bool,
            }]],
            vec![],
        ),
        HirPattern::NoneSingleton => (
            vec![vec![MirExpr::Compare {
                op: CmpOpKind::Eq,
                left: Box::new(subj.clone()),
                right: Box::new(MirExpr::Name {
                    name: "None".to_string(),
                    ty: Ty::None,
                }),
                ty: Ty::Bool,
            }]],
            vec![],
        ),
        HirPattern::Sequence(sub_pats) => {
            lower_sequence_conds(subj, sub_pats, None, scopes, classes, current_class)
        }
        HirPattern::SequenceStar(sub_pats, rest) => lower_sequence_conds(
            subj,
            sub_pats,
            rest.as_ref(),
            scopes,
            classes,
            current_class,
        ),
        HirPattern::Mapping(pairs, rest) => {
            lower_mapping_conds(subj, pairs, rest.as_ref(), scopes, classes, current_class)
        }
        HirPattern::Class {
            class_name,
            positional,
            keyword,
        } => lower_class_conds(
            subj,
            class_name,
            positional,
            keyword,
            scopes,
            classes,
            current_class,
        ),
        HirPattern::Or(subs) => {
            let mut all_alts = Vec::new();
            let mut all_bindings = Vec::new();
            for sub in subs {
                let (alts, b) = lower_pattern_conds(subj, sub, scopes, classes, current_class);
                all_alts.extend(alts);
                if all_bindings.is_empty() {
                    all_bindings = b;
                }
            }
            (all_alts, all_bindings)
        }
        HirPattern::As(inner, name) => {
            let (alts, bindings) = lower_pattern_conds(subj, inner, scopes, classes, current_class);
            let mut all = bindings;
            all.push((name.clone(), subj.clone()));
            (alts, all)
        }
    }
}

/// Lowers a sequence pattern into conditions: a length check plus
/// per-element sub-pattern conditions.
fn lower_sequence_conds(
    subj: &MirExpr,
    sub_pats: &[HirPattern],
    rest: Option<&String>,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> (Vec<Vec<MirExpr>>, Vec<(String, MirExpr)>) {
    let fixed = sub_pats.len();
    let len_cond = MirExpr::Compare {
        op: if rest.is_some() {
            CmpOpKind::GtE
        } else {
            CmpOpKind::Eq
        },
        left: Box::new(MirExpr::Call {
            callee: "len".to_string(),
            args: vec![subj.clone()],
            ty: Ty::Int,
        }),
        right: Box::new(MirExpr::IntLiteral(fixed as i64)),
        ty: Ty::Bool,
    };
    let mut conds = vec![len_cond];
    let mut bindings = Vec::new();
    for (i, sub_pat) in sub_pats.iter().enumerate() {
        let elem = MirExpr::Subscript {
            base: Box::new(subj.clone()),
            index: Box::new(MirExpr::IntLiteral(i as i64)),
        };
        let (alts, b) = lower_pattern_conds(&elem, sub_pat, scopes, classes, current_class);
        for alt in alts {
            conds.extend(alt);
        }
        bindings.extend(b);
    }
    if let Some(rest_name) = rest {
        bindings.push((rest_name.clone(), subj.clone()));
    }
    (vec![conds], bindings)
}

/// Lowers a mapping pattern into per-key-value check conditions.
fn lower_mapping_conds(
    subj: &MirExpr,
    pairs: &[(HirExpr, HirPattern)],
    rest: Option<&String>,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> (Vec<Vec<MirExpr>>, Vec<(String, MirExpr)>) {
    let mut conds = Vec::new();
    let mut bindings = Vec::new();
    for (key_expr, val_pat) in pairs {
        let key_lowered = lower_expr(key_expr, scopes, classes, current_class);
        let val = MirExpr::DictGet {
            dict: Box::new(subj.clone()),
            key: Box::new(key_lowered),
        };
        let (alts, b) = lower_pattern_conds(&val, val_pat, scopes, classes, current_class);
        for alt in alts {
            conds.extend(alt);
        }
        bindings.extend(b);
    }
    if let Some(rest_name) = rest {
        bindings.push((rest_name.clone(), subj.clone()));
    }
    (vec![conds], bindings)
}

/// Lowers a class pattern into per-attribute check conditions.
#[allow(clippy::expect_fun_call)]
fn lower_class_conds(
    subj: &MirExpr,
    class_name: &str,
    positional: &[HirPattern],
    keyword: &[(String, HirPattern)],
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> (Vec<Vec<MirExpr>>, Vec<(String, MirExpr)>) {
    let class_def = classes.get(class_name).expect(&format!(
        "pycc_mir: internal error: class `{class_name}` has no registered HirClassDef -- \
         pycc_types::check should have rejected this HIR before it reached pycc_mir"
    ));
    let flat_attrs = mro_attrs(class_def, classes);
    let mut conds = Vec::new();
    let mut bindings = Vec::new();
    for (i, sub_pat) in positional.iter().enumerate() {
        let (_, ty) = flat_attrs.get(i).expect(&format!(
            "pycc_mir: internal error: class `{class_name}` has fewer attributes than \
             positional patterns -- pycc_types::check should have rejected this HIR \
             before it reached pycc_mir"
        ));
        let attr_val = MirExpr::AttrGet {
            base: Box::new(subj.clone()),
            slot: i,
            ty: ty.clone(),
        };
        let (alts, b) = lower_pattern_conds(&attr_val, sub_pat, scopes, classes, current_class);
        for alt in alts {
            conds.extend(alt);
        }
        bindings.extend(b);
    }
    for (attr_name, sub_pat) in keyword {
        let (slot, (_, ty)) = flat_attrs
            .iter()
            .enumerate()
            .find(|(_, (name, _))| name == attr_name)
            .expect(&format!(
                "pycc_mir: internal error: attribute `{attr_name}` not declared on class \
                 `{class_name}` or any base in its MRO -- pycc_types::check should have \
                 rejected this HIR before it reached pycc_mir"
            ));
        let attr_val = MirExpr::AttrGet {
            base: Box::new(subj.clone()),
            slot,
            ty: ty.clone(),
        };
        let (alts, b) = lower_pattern_conds(&attr_val, sub_pat, scopes, classes, current_class);
        for alt in alts {
            conds.extend(alt);
        }
        bindings.extend(b);
    }
    (vec![conds], bindings)
}

/// Rewrites an attribute access `base.attr` (where `base` names an enum
/// class and `attr` is one of its declared members, i.e. `attr` reads a
/// member on the enum class) to `MirExpr::Name` reading the synthetic
/// `<Class>.<Member>.enum_member` global. Returns `None` if `base` is not
/// an enum class name or `attr` is not one of its members. Extracted from
/// `lower_expr` to isolate the enum-specific code paths (see
/// cargo-llvm-cov#276 for the coverage instantiation issue).
pub(super) fn try_lower_enum_member_attr(
    base: &HirExpr,
    attr: &str,
    classes: &HashMap<String, HirClassDef>,
) -> Option<MirExpr> {
    if let HirExpr::Name(class_name) = base
        && let Some(class_def) = classes.get(class_name.as_str())
        && !class_def.enum_members.is_empty()
        && class_def.enum_members.iter().any(|(name, _)| name == attr)
    {
        return Some(MirExpr::Name {
            name: format!("{class_name}.{attr}.enum_member"),
            ty: Ty::Instance(Box::new(class_name.clone())),
        });
    }
    None
}
